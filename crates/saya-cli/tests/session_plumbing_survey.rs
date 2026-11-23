//! The session-plumbing survey X3 drew, after PT4 acted on it: what is live,
//! and what is gone.
//!
//! Step 1 of X3's brief surveyed `SessionUniverse|session::universe|`
//! `session::runtime|` the request body`|` the session's own definitions. It
//! came back mixed. The live half still stands: the terminal launch composes
//! `SessionRuntime`/`SessionUniverse` (`universe().tasks()`,
//! `sync_host_posture`), `front/tui/direct_trust.rs` holds a
//! `SessionRuntime`, and the chat's mode surface reads the permission
//! families. The dead half is deleted: the per-turn request body, the old turn
//! engine it was the only production caller of, and the session's own
//! write-shaped definitions whose only reader was the engine's
//! advertisement — every pin that ran through them having been twinned on the
//! live chat path first (PT1–PT6).
//!
//! These guards pin the two statements that shape the result: the plumbing the
//! live fronts compose stays, its live readers stay, and the deleted half is
//! gone and unnamed. They read the tree the way `tui_trunk_removed` and
//! `turn_config_single_home` do — paths and source text — because each
//! statement is about the layout. Needles are spelled in halves so this file
//! never matches itself.

use std::path::{Path, PathBuf};

/// The modules the survey found live, and therefore kept. A live front imports
/// each of these on a path this crate builds.
const KEPT: &[&str] = &[
    "interactive/session/universe/mod.rs",
    "interactive/session/runtime/mod.rs",
    "interactive/session/permissions/grant_surface.rs",
    "interactive/session/permissions/grant_families.rs",
    concat!("interactive/session/line_chat/open", ".rs"),
    "agent/turn_inputs.rs",
];

/// The deleted half: the request body, the old turn engine, and the session's
/// own definitions (whose only production reader was the engine's
/// advertisement).
const GONE: &[&str] = &[
    concat!("interactive/session", "_request.rs"),
    concat!("agent/runtime", ".rs"),
    concat!("agent/runtime", "/"),
];

/// Each kept module's live caller, and the needle that proves the read: the
/// terminal launch's session, the TUI's trust store, recovery's attach, the
/// chat's mode surface over the permission families, and the live turns'
/// prompt preparation.
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
    ("interactive/session/line_chat/open.rs", "prepare_turn"),
];

/// The old turn engine's one production entry point, and the module path its
/// callers used: neither may be named again.
const ENGINE_ENTRY: &str = concat!("run_prompt_with", "_sink_and_outcomes");
const ENGINE_PATH: &str = concat!("agent::", "runtime::");
const REQUEST_BODY: &str = concat!("session", "_request::");

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

/// The dead half is gone: the body and its request entry are not on disk, and
/// no line names the engine's entry or its module path any more. A file
/// restored from history — the mutation this guard was written for — is named
/// here.
#[test]
fn the_old_body_and_its_request_entry_are_gone() {
    let root = crate_root();
    let mut offenders: Vec<String> = GONE
        .iter()
        .filter(|path| root.join("src").join(path).exists())
        .map(|path| format!("src/{path} still exists"))
        .collect();
    for (path, text) in sources() {
        for (index, line) in text.lines().enumerate() {
            let names_it = line.contains(ENGINE_ENTRY)
                || line.contains(ENGINE_PATH)
                || line.contains(REQUEST_BODY);
            if names_it {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the old request body, the engine it was the only production caller of \
         and the session's own definitions are deleted; nothing may name them \
         again ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
