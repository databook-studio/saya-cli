//! The prompt-config types have one home, and no live front reaches into the
//! old turn engine.
//!
//! X2 puts the one definition of `AgentRuntimeError`, `PromptOverrides` and
//! `query_data_allowed_for_endpoint` (with its `query_data_allowed` helper and
//! `effective_ai`) in `agent/turn_config.rs`, and moves the leaves a live
//! front still reads — `with_next_step`, the provider advice,
//! `project_untrusted_context` — next to the callers that read them. Three
//! things must not drift afterwards:
//!
//! - a second home, or a re-export alias, for the moved items: the old
//!   engine's module path must stop being a way to reach them;
//! - a live front naming the old engine's module at all: the job engine, the
//!   chat layer and `saya ask` read the leaves from where they now live;
//! - the old turn body being deleted while the users it is kept for still
//!   drive it.
//!
//! The guard reads the tree the way `tui_trunk_removed` does — paths and
//! source text — because each statement is about the layout the brief asked
//! for. Needles are spelled in halves so this file never matches itself.

use std::path::{Path, PathBuf};

/// Each moved definition, and the one file that may carry it.
const MOVED_DEFINITIONS: &[(&str, &str)] = &[
    (
        concat!("pub(crate) enum Agent", "RuntimeError"),
        "agent/turn_config.rs",
    ),
    (
        concat!("pub(crate) struct Prompt", "Overrides"),
        "agent/turn_config.rs",
    ),
    (
        concat!("pub(crate) fn query_data_allowed", "_for_endpoint"),
        "agent/turn_config.rs",
    ),
];

/// The names the old engine's re-export block carried. No line may reach one
/// of them through `runtime::`.
const MOVED_NAMES: &[&str] = &[
    concat!("Agent", "RuntimeError"),
    concat!("Prompt", "Overrides"),
    concat!("query_data_allowed", "_for_endpoint"),
    concat!("query_data_allowed", "("),
    concat!("effective", "_ai"),
];

/// The files that may still name the old turn body: the old per-turn request
/// body X3 deletes, the old-session tests that drive the body as their
/// harness, and the context tests that do the same. Every entry is list (b) in
/// `REPORT.md`; deleting the body needs each re-pointed at the chat path first.
const OLD_ENGINE_FILES: &[&str] = &[
    "interactive/session_request.rs",
    "interactive/session/universe/tests/shell_background.rs",
    "interactive/session/universe/tests/shell_turn.rs",
    "interactive/session/universe/tests/workspace_write_effects.rs",
    "interactive/session/universe/tests/grant_recompute_and_host_refusal.rs",
    "interactive/session/continuation/record_block_tests.rs",
    "agent/context/preferences_tests.rs",
    "agent/context/sources_tests.rs",
];

/// The body's own module and its test modules and fixtures, by prefix: the
/// body is what these exist to drive.
const OLD_ENGINE_PREFIXES: &[&str] = &["agent/runtime/", "agent/runtime_"];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every first-party source under this crate, tests included: a call site is a
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

fn is_old_engine_user(relative: &str) -> bool {
    OLD_ENGINE_FILES.contains(&relative)
        || OLD_ENGINE_PREFIXES
            .iter()
            .any(|prefix| relative.starts_with(prefix))
}

/// Every definition of a moved item, with the file that carries it.
fn definitions_of(needle: &str) -> Vec<String> {
    sources()
        .into_iter()
        .filter(|(_, text)| text.contains(needle))
        .map(|(path, _)| path)
        .collect()
}

/// The moved items have exactly one home each, and no second file re-exports
/// them under the old engine's path: one definition, one import path.
#[test]
fn the_moved_items_have_one_home() {
    let mut offenders: Vec<String> = MOVED_DEFINITIONS
        .iter()
        .flat_map(|(needle, home)| {
            definitions_of(needle)
                .into_iter()
                .filter(move |path| path != home)
                .map(move |path| format!("{path} defines `{needle}` outside {home}"))
        })
        .collect();
    // A re-export is the other way to keep the old path alive: the item is
    // defined once and reachable twice.
    for (path, text) in sources() {
        if path == "agent/turn_config.rs" {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            let reexports = line.contains("pub(crate) use") || line.contains("pub use");
            if reexports && line.contains(concat!("turn_", "config::")) {
                offenders.push(format!("{}:{}: {}", path, index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the moved prompt-config items must have one home, \
         `agent/turn_config.rs`, and no re-export alias ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// Nothing reaches a moved item through the old engine's module: every caller
/// imports it from `agent::turn_config`.
#[test]
fn no_caller_reaches_the_moved_items_through_the_old_engine() {
    let mut offenders = Vec::new();
    for (path, text) in sources() {
        if path == "agent/turn_config.rs" {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            if !line.contains(concat!("runtime", "::")) {
                continue;
            }
            if MOVED_NAMES.iter().any(|name| line.contains(name)) {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these lines reach a moved prompt-config item through the old turn \
         engine, but its one home is `agent::turn_config` ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// No file outside the old engine's kept users names it — not the job engine,
/// not the chat layer, not `saya ask`. The full path is checked everywhere; the
/// crate-relative spelling only under `agent/`, where it can only mean the old
/// turn engine (every other `super::runtime` is another module's own).
#[test]
fn no_live_front_names_the_old_engine() {
    let full = concat!("agent::", "runtime::");
    let relative = concat!("super::", "runtime::");
    let mut offenders = Vec::new();
    for (path, text) in sources() {
        if is_old_engine_user(&path) {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            let names_it =
                line.contains(full) || (path.starts_with("agent/") && line.contains(relative));
            if names_it {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these lines name the old turn engine from a live path; the leaves it \
         still provided are moved next to their callers ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The body is kept, so its kept users must still be there: deleting one of
/// them (rather than re-pointing it) would drop the behaviour it pins.
#[test]
fn the_kept_users_still_exist() {
    let root = crate_root();
    let mut missing: Vec<&str> = OLD_ENGINE_FILES
        .iter()
        .copied()
        .filter(|path| !root.join("src").join(path).exists())
        .collect();
    let paths: Vec<String> = sources().into_iter().map(|(path, _)| path).collect();
    missing.extend(
        OLD_ENGINE_PREFIXES
            .iter()
            .copied()
            .filter(|prefix| !paths.iter().any(|path| path.starts_with(prefix))),
    );
    assert!(
        missing.is_empty(),
        "the old turn body is kept for these users, but they are gone ({} \
         path(s)):\n{}",
        missing.len(),
        missing.join("\n")
    );
}
