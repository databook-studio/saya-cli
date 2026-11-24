//! The prompt-config types have one home, and the old turn engine is gone.
//!
//! X2 put the one definition of `AgentRuntimeError`, `PromptOverrides` and
//! `query_data_allowed_for_endpoint` in `agent/turn_config.rs` and moved the
//! leaves a live front reads next to their callers; PT4 deleted the old body,
//! its request entry, its suite and every old pin that had a live twin. The
//! statements that must not drift afterwards: one home and no re-export alias
//! for the moved items, no line naming the old engine's module, and the body
//! staying gone and unnamed. Paths and source text, like `tui_trunk_removed`;
//! needles are spelled in halves so this file never matches itself.

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

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The crate-relative spelling of a path: `/` separators, and no `src/`
/// prefix, so a module and its file agree on one name on every host.
fn crate_relative(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.strip_prefix("src/").unwrap_or(&path).to_owned()
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
            let relative = crate_relative(&relative);
            let text = std::fs::read_to_string(&path).ok()?;
            Some((relative, text))
        })
        .collect()
}

/// Every definition of a moved item, with the file that carries it.
fn definitions_of(needle: &str) -> Vec<String> {
    sources()
        .into_iter()
        .filter(|(_, text)| text.contains(needle))
        .map(|(path, _)| path)
        .collect()
}

/// A path this crate reads is spelled with `src/` and `/` separators wherever
/// the checkout was made: a Windows separator names the same file.
#[test]
fn a_windows_path_is_read_as_its_unix_spelling() {
    assert_eq!(
        crate_relative("src\\agent\\turn_config.rs"),
        "agent/turn_config.rs",
        "a Windows path names the one home a moved item has"
    );
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

/// No file names the old engine at all — not the job engine, not the chat
/// layer, not `saya ask`, and not a comment explaining what used to be there.
/// With the body deleted there is no kept user left, so there is no exclusion.
#[test]
fn no_live_front_names_the_old_engine() {
    let full = concat!("agent::", "runtime::");
    let relative = concat!("super::", "runtime::");
    let mut offenders = Vec::new();
    for (path, text) in sources() {
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
        "these lines name the old turn engine, which is deleted; the leaves it \
         still provided live next to their callers ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
