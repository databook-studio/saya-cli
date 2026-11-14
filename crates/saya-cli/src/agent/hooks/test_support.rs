//! Real hook engines for tests: one event, stated as the product's own
//! `[hooks]` table.
//!
//! Each is built through the same resolution and the same [`Hooks::new`] the
//! product uses — a `[hooks]` table parsed from TOML, resolved by
//! `saya-config`, run through the harness shell — so a test exercises the
//! engine itself rather than a stub that could agree with one surface while
//! the engine disagrees with all of them.

use std::path::Path;
use std::sync::Arc;

use saya_harness::runner::HostPolicy;

use super::{HookCaller, Hooks};

/// Escapes `command` as one TOML basic string.
fn toml_string(command: &str) -> String {
    command.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `[hooks]` table stating one `PreToolUse` hook that writes `reason` to
/// stderr and exits `2`. Both escapings are real: the reason is one shell word
/// whatever it holds, and the command is one TOML basic string.
fn blocking_table(reason: &str) -> String {
    let command = format!(
        "printf '%s' '{}' >&2; exit 2",
        reason.replace('\'', "'\\''")
    );
    format!(
        "[[hooks.pre_tool_use]]\ncommand = \"{}\"\n",
        toml_string(&command)
    )
}

/// The `[hooks]` table stating one `Stop` hook whose command is `command`.
fn stop_table(command: &str) -> String {
    format!("[[hooks.stop]]\ncommand = \"{}\"\n", toml_string(command))
}

/// The engine `table` resolves to, over `root` for `caller`.
fn engine_from(table: &str, root: &Path, caller: HookCaller) -> Arc<Hooks> {
    let file =
        saya_config::ConfigFile::from_toml(table).expect("the fixture's hook table is well formed");
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default()).with_user(file),
    )
    .expect("the fixture's config resolves");
    assert!(
        !resolved.hooks.is_empty(),
        "the fixture must state a hook, or the test proves nothing"
    );
    Arc::new(
        Hooks::new(
            resolved.hooks,
            root,
            HostPolicy::default(),
            caller,
            saya_agent::CancellationToken::new(),
        )
        .expect("a host-only shell over the test root"),
    )
}

/// An engine over `root` holding exactly one `PreToolUse` hook, watching every
/// tool, that blocks with `reason`.
pub(crate) fn blocking_hooks(reason: &str, root: &Path, caller: HookCaller) -> Arc<Hooks> {
    engine_from(&blocking_table(reason), root, caller)
}

/// An engine over `root` holding exactly one `Stop` hook whose command is
/// `command`, so a test can record that a loop's end fired it.
pub(crate) fn stop_hooks(command: &str, root: &Path, caller: HookCaller) -> Arc<Hooks> {
    engine_from(&stop_table(command), root, caller)
}
