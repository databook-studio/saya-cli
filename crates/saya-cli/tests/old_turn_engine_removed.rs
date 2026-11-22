//! The old turn engine is gone, and nothing names it again.
//!
//! Split from `turn_config_single_home` when PT4 deleted the body: that guard
//! still watches where the moved prompt-config items live, this one watches
//! that the deleted half stays deleted. A file restored from history — the
//! mutation this guard exists for — is named here. Needles are spelled in
//! halves so this file never matches itself.

use std::path::{Path, PathBuf};

const REMOVED: &[&str] = &[
    concat!("interactive/session", "_request.rs"),
    concat!("agent/runtime", ".rs"),
    concat!("agent/runtime", "_budget_tests.rs"),
    concat!("agent/runtime", "_budget_tests"),
    concat!("agent/runtime", "_metrics_tests.rs"),
    concat!("agent/runtime", "_preflight_tests.rs"),
    concat!("agent/runtime", "_preflight_tests"),
    concat!("agent/runtime", "_preflight_second_tests.rs"),
    concat!("agent/runtime", "_tests.rs"),
    concat!("agent/runtime", "_tests"),
    concat!("agent/runtime", "_timer_tests.rs"),
    concat!("agent/runtime", "_timer_tests"),
    concat!("agent/runtime", "/"),
];

/// The names no line in the crate may carry again: the body's module path, its
/// production entry point, and its request entry.
///
/// The old session's own definitions are *not* on this list: their production
/// reader (the engine's advertisement) is gone, but the approval, deny and
/// prompt-facts tests still need each old door's exact shape. They are kept as
/// `#[cfg(test)]`-only fixtures, which `the_old_definitions_are_test_only`
/// pins.
const REMOVED_NAMES: &[&str] = &[
    concat!("agent::", "runtime::"),
    concat!("run_prompt_with", "_sink_and_outcomes"),
    concat!("run_prompt_with", "_sink("),
    concat!("session", "_request::"),
    concat!("interactive/session", "_request.rs"),
];

/// The relative spelling of the body's module path. Only `agent/` files can
/// mean this one by `super::runtime`: `coding::runtime` and `config::runtime`
/// are other modules with the same last segment.
const REMOVED_RELATIVE: &str = concat!("super::", "runtime::");

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

/// The body is gone, and so is every name that pointed at it. A file restored
/// from history (or a comment that still speaks of the request entry) fails
/// here.
#[test]
fn the_old_engine_is_deleted() {
    let root = crate_root();
    let mut offenders: Vec<String> = REMOVED
        .iter()
        .filter(|path| root.join("src").join(path).exists())
        .map(|path| format!("src/{path} still exists"))
        .collect();
    for (path, text) in sources() {
        for (index, line) in text.lines().enumerate() {
            let names_removed = REMOVED_NAMES.iter().any(|name| line.contains(name))
                || (path.starts_with("agent/") && line.contains(REMOVED_RELATIVE));
            if names_removed {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the old turn body, its request entry, its suite and the session's own \
         definitions are deleted, and nothing may name them again ({} \
         offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The two surviving definitions modules are fixtures, not a surface: the crate
/// declares both under `#[cfg(test)]`, so no shipping build advertises them.
#[test]
fn the_old_definitions_are_test_only() {
    let mod_rs = std::fs::read_to_string(crate_root().join("src/interactive/mod.rs"))
        .expect("the interactive module file reads");
    let mut offenders = Vec::new();
    for name in [
        ["session", "_definitions"].concat(),
        ["session", "_file_definitions"].concat(),
    ] {
        let declaration = format!("pub(crate) mod {name};");
        let Some(index) = mod_rs.lines().position(|line| line.trim() == declaration) else {
            offenders.push(format!(
                "{name} is no longer declared in interactive/mod.rs"
            ));
            continue;
        };
        let lines: Vec<&str> = mod_rs.lines().take(index).collect();
        let gated = lines
            .iter()
            .rev()
            .find(|line| line.trim().starts_with("#["))
            .is_some_and(|line| line.trim() == "#[cfg(test)]");
        if !gated {
            offenders.push(format!("{name} is declared without #[cfg(test)] above it"));
        }
    }
    assert!(
        offenders.is_empty(),
        "the old session definitions may exist only as test fixtures ({} \
         offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
