//! The contained walk under `glob` and `grep`: one bounded descent over the
//! workspace tree. It lives in the harness — not beside the tools that call
//! it — because walking the tree IS containment work: every directory is
//! entered through `Workspace::list`, so every path the walk touches goes
//! through the same argument validation, symlink refusal, and root-prefix
//! check as any direct call. A second place that resolved paths would be a
//! second place those rules could be wrong.

use crate::HarnessError;

use super::contain::{EntryKind, Workspace};

impl Workspace {
    /// Walks the workspace depth-first, calling `visit` for every real file
    /// and directory relative to the root; the root itself is not visited.
    /// Symlinks are neither visited nor descended into: `list` reports them
    /// as [`EntryKind::Symlink`] and they are dropped here, so a link can
    /// never carry the walk (or a search over it) across the boundary.
    /// Every entry is checked once more against the containment layer before
    /// it is reported, so a candidate is always a path the layer would accept
    /// as an argument. At most `max_visited` entries are visited overall —
    /// the (max+1)-th refuses with a typed bounds error before any per-entry
    /// work happens on it, because a truncated search result would read as
    /// proof of absence. Enumeration itself is bounded the same way: one
    /// directory holding more than `max_visited` entries refuses on its own
    /// listing.
    pub(crate) fn walk(
        &self,
        max_visited: usize,
        visit: &mut dyn FnMut(&str, EntryKind, u64) -> Result<(), HarnessError>,
    ) -> Result<(), HarnessError> {
        let mut pending = vec![String::new()];
        let mut visited = 0usize;
        while let Some(dir) = pending.pop() {
            let entries = self.list(&dir, max_visited)?;
            for entry in entries {
                visited += 1;
                if visited > max_visited {
                    return Err(HarnessError::BoundsExceeded {
                        path: dir,
                        found: visited as u64,
                        max: max_visited as u64,
                    });
                }
                let rel = if dir.is_empty() {
                    entry.name
                } else {
                    format!("{dir}/{}", entry.name)
                };
                match entry.kind {
                    EntryKind::Symlink => {}
                    EntryKind::Dir => {
                        if self.admits(&rel) {
                            visit(&rel, EntryKind::Dir, entry.size)?;
                            pending.push(rel);
                        }
                    }
                    kind => {
                        if self.admits(&rel) {
                            visit(&rel, kind, entry.size)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether the containment layer itself would accept `rel` as a path
    /// argument. The names it refuses are skipped, never reported and never
    /// fatal: a `.git` name is hygiene-denied content the search should not
    /// serve, a name the argument validator cannot parse (an odd but legal
    /// filesystem name) is one no tool could ever open through the layer, and
    /// a not-found is a file the walk raced away underneath itself. Skipped
    /// rather than served — the walk's guarantee is that everything it hands
    /// out is contained, not that the filesystem is tidy.
    fn admits(&self, rel: &str) -> bool {
        self.target(rel, false).is_ok()
    }
}
