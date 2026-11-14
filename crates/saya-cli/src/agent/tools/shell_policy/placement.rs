//! Where a shell runs: the workspace root its children are confined to, and
//! the scratch tree a helper's own shell gets.

use std::path::Path;

/// Where a shell runs: the workspace root its children are confined to, and —
/// for a helper's own shell — the private scratch `HOME`/`TMPDIR` its
/// sandboxed lane gets instead of the checkout root. The root's shell has
/// no scratch of its own: its `HOME` is the checkout.
pub(crate) struct ShellPlacement<'a> {
    /// The workspace root the shell's workdir is confined to.
    pub(crate) root: &'a Path,
    /// The scratch `HOME`/`TMPDIR` a sandboxed child gets, when it is not the
    /// root itself.
    pub(crate) scratch: Option<(&'a Path, &'a Path)>,
    /// The job's private state directory, where a result too large for the
    /// model's inline bound saves its full captured output. Never inside the
    /// workspace: the shell refuses such a directory rather than writing the
    /// model's escape hatch into the user's tree.
    pub(crate) spool: Option<&'a Path>,
}

impl<'a> ShellPlacement<'a> {
    /// The root's own placement: the checkout, no scratch.
    pub(crate) fn root(root: &'a Path) -> Self {
        Self {
            root,
            scratch: None,
            spool: None,
        }
    }

    /// A helper's placement: the shared working checkout, with this helper's
    /// own scratch tree.
    pub(crate) fn helper(root: &'a Path, scratch: (&'a Path, &'a Path)) -> Self {
        Self {
            root,
            scratch: Some(scratch),
            spool: None,
        }
    }

    /// Saves the full captured output of a large result under `spool`.
    pub(crate) fn with_spool(mut self, spool: &'a Path) -> Self {
        self.spool = Some(spool);
        self
    }
}
