//! The `/recovery` operation layer: one `run_recovery` every surface runs
//! over the session's own recovery handle — its store, id and shared flag;
//! callers add only their emit.

use super::{
    ChangeItem, ChangesView, diff_change, list_changes, render_changes, render_diff_outcome,
    render_restore, restore_change,
};
use crate::interactive::session_recovery::RecoveryHandle;
use saya_harness::workspace::Workspace;
use saya_store::FsSessionStore;

/// The retention text `/recovery on` says, byte for byte, on every surface.
const ON_TEXT: &str = "Recovery is on. From the next file edit, workspace_write and workspace_edit keep the file's previous contents so you can undo an edit. The copies are private to you (owner-only files under the saya sessions directory, outside your project), limited to 128 edits, 64 MiB and 8 MiB per file, and are deleted with the session (/sessions delete). Recovery covers direct file-tool edits only — never commands, downloads, scratch tables or databases — and files that look like credentials or secrets are edited without a copy. Edits made before this point were not recorded. Use /recovery list to see them and /recovery restore <id> to undo one.";

/// One `/recovery` invocation, already parsed: sub-word plus typed id.
/// Nominally `pub` like `NotesCommand` — `SlashCommand` carries it, and
/// the enclosing module keeps the effective visibility crate-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryCommand {
    Status,
    On,
    Off,
    List,
    Diff(String),
    Restore(String),
}

/// Runs one `/recovery` command over the session's own recovery handle:
/// its store, id and shared flag — the recorder's parts. `true` is a
/// success; `false` names a refusal; nothing changed but flag and restore.
pub(crate) fn run_recovery(
    command: RecoveryCommand,
    handle: &RecoveryHandle,
    workspace: Option<&Workspace>,
    now_ms: i64,
) -> (bool, String) {
    let (store, session_id, _) = handle.parts();
    match command {
        RecoveryCommand::Status => match list_changes(&store, &session_id, workspace) {
            Ok(view) => (true, status_text(&view, handle.is_enabled(), workspace)),
            Err(error) => (false, format!("could not list recorded edits: {error}")),
        },
        RecoveryCommand::On if workspace.is_none() => (
            false,
            "No workspace is bound, so there are no file edits to record. Launch inside a \
             git worktree or pass --workspace <dir>."
                .to_owned(),
        ),
        RecoveryCommand::On if handle.is_enabled() => (true, "Recovery is already on.".to_owned()),
        RecoveryCommand::On => {
            handle.set_enabled(true);
            (true, ON_TEXT.to_owned())
        }
        RecoveryCommand::Off => {
            handle.set_enabled(false);
            (true, "Recovery is off.".to_owned())
        }
        RecoveryCommand::List => match list_changes(&store, &session_id, workspace) {
            Ok(view) => (true, render_changes(&view, now_ms)),
            Err(error) => (false, format!("could not list recorded edits: {error}")),
        },
        RecoveryCommand::Diff(typed) => match resolve_typed(&store, &session_id, workspace, &typed)
        {
            Ok(id) => render_diff_outcome(&diff_change(&store, &session_id, workspace, &id)),
            Err(message) => (false, message),
        },
        RecoveryCommand::Restore(typed) => {
            match resolve_typed(&store, &session_id, workspace, &typed) {
                Ok(id) => {
                    let (ok, mut message) =
                        render_restore(&restore_change(&store, &session_id, workspace, &id));
                    if ok {
                        message.push_str(
                            "\nRestoring a file does not undo commands, downloads or \
                             database effects.",
                        );
                    }
                    (ok, message)
                }
                Err(message) => (false, message),
            }
        }
    }
}

/// The status line: flag, quota use, off-state hint, no-workspace note.
fn status_text(view: &ChangesView, enabled: bool, workspace: Option<&Workspace>) -> String {
    let mut text = format!(
        "Recovery is {}. {} recorded edit(s) this session ({} KiB of {} MiB, {} of {}).",
        if enabled { "on" } else { "off" },
        view.usage.edits,
        view.usage.retained_bytes.div_ceil(1024),
        view.max_bytes / (1024 * 1024),
        view.usage.edits,
        view.max_edits,
    );
    if !enabled && view.usage.edits > 0 {
        text.push_str(" Recorded edits stay available: /recovery list.");
    }
    if workspace.is_none() {
        text.push_str(" No workspace is bound.");
    }
    text
}

/// Lists the edits and resolves the typed id against the items, so the
/// store never sees an unresolved string.
fn resolve_typed(
    store: &FsSessionStore,
    session_id: &str,
    workspace: Option<&Workspace>,
    typed: &str,
) -> Result<String, String> {
    let view = list_changes(store, session_id, workspace)
        .map_err(|error| format!("could not list recorded edits: {error}"))?;
    resolve_id(&view.items, typed)
}

/// Resolves the typed id against the listing's items (full id or prefix).
pub fn resolve_id(items: &[ChangeItem], typed: &str) -> Result<String, String> {
    if items.iter().any(|item| item.id == typed) {
        return Ok(typed.to_owned());
    }
    let Some(rest) = typed.strip_prefix("ck-") else {
        return Err("give the id as /recovery list shows it: ck-… or a unique prefix".to_owned());
    };
    if rest.len() < 6 {
        return Err(format!(
            "{typed} is too short to identify a checkpoint: give at least 6 characters \
             after ck- (see /recovery list)"
        ));
    }
    let matches: Vec<&str> = items
        .iter()
        .map(|item| item.id.as_str())
        .filter(|id| id.starts_with(typed))
        .collect();
    match matches.as_slice() {
        [only] => Ok((*only).to_owned()),
        [] => Err(format!(
            "{typed} matches no recorded edit this session (see /recovery list)"
        )),
        many => Err(format!(
            "{typed} matches {} recorded edits; give more characters to pick one (see \
             /recovery list)",
            many.len()
        )),
    }
}
