//! The one place outside the workspace `read` and `grep` may reach: this
//! job's own shell-output spool. A path is a spool path only when it resolves
//! under the directory the tool was composed with — another session's spool
//! is a different directory and stays unreachable. The spool is opened as its
//! own `Workspace`, so every containment rule (no-follow, component walk,
//! bounds) still runs, and the workspace root is never widened.

use std::path::{Path, PathBuf};

use saya_harness::workspace::Workspace;

use super::super::DatabaseTools;

pub(super) struct SpoolTarget {
    /// The spool directory opened as its own workspace root.
    pub(super) workspace: Workspace,
    /// The requested path relative to that root.
    pub(super) relative: String,
    /// The canonical spool root, used to name results the model can pass
    /// back to `read` or `grep`.
    pub(super) base: PathBuf,
}

impl DatabaseTools {
    /// Resolves `path` under this job's spool directory, when it names one.
    /// Canonicalising both sides refuses a symlink out of the spool and any
    /// path outside it.
    pub(super) fn spool_target(&self, path: &str) -> Option<SpoolTarget> {
        let dir = self.spool_dir.as_deref()?;
        let base = std::fs::canonicalize(dir).ok()?;
        let canonical = std::fs::canonicalize(Path::new(path)).ok()?;
        let relative = canonical.strip_prefix(&base).ok()?;
        let relative = relative.to_str()?.to_owned();
        let workspace = Workspace::open(&base).ok()?;
        Some(SpoolTarget {
            workspace,
            relative,
            base,
        })
    }
}
