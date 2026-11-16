//! Whether the workspace holds a notebook, for the surfaces that offer
//! `notebook_edit` only where one exists.
//!
//! The decision is the harness's bounded presence check
//! ([`Workspace::holds_file_extension`]): the same contained,
//! symlink-refusing, gitignore-aware walk the searches use, stopped at the
//! first `.ipynb` name under a named entry bound and deadline. A workspace
//! the check found nothing in — or one it could not finish inside its bounds
//! — offers no notebook edit: the tool is advertised only on a definite yes,
//! so a workspace with no notebook costs the model no context for it, and a
//! call while it is not offered is refused as an unknown tool. `read` of a
//! `.ipynb` keeps working either way.

use saya_agent::ToolDefinition;
use saya_harness::workspace::{PresenceBounds, Workspace};

/// The extension a notebook carries, without the dot.
const NOTEBOOK_EXTENSION: &str = "ipynb";

/// Whether `workspace` holds at least one notebook. `None` — no workspace
/// bound — holds none.
pub(crate) fn present(workspace: Option<&Workspace>) -> bool {
    workspace.is_some_and(|workspace| {
        workspace
            .holds_file_extension(NOTEBOOK_EXTENSION, PresenceBounds::default())
            .found
    })
}

/// Drops the notebook edit from a composed list. A surface whose list is
/// fixed for its run — a run step, a helper — calls this once, at
/// composition, when [`present`] says the workspace holds no notebook.
///
/// The definition is *removed*, not merely refused at the door, so an absent
/// notebook costs the model no context — the same hidden-not-advertised rule
/// the workspace-write permit follows. A surface that re-reads its list every
/// turn does not call this: it keeps the definition and filters it at
/// advertisement, so a notebook created during the run can still make the
/// tool appear.
pub(crate) fn drop_notebook_edit(definitions: &mut Vec<ToolDefinition>) {
    definitions.retain(|definition| definition.name != "notebook_edit");
}
