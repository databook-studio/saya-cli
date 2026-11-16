//! One descriptor-backed DFS used by strict legacy and controlled partial search.
use super::{EntryKind, Workspace, search_directory::Directory};
use crate::HarnessError;
use std::time::SystemTime;

#[derive(Clone, Copy)]
pub(super) enum WalkAction {
    Continue,
    Prune,
    Stop,
}

pub(super) trait WalkVisitor {
    fn advance(&mut self) -> bool {
        true
    }
    fn seen(&mut self) -> Result<bool, HarnessError>;
    fn enter(&mut self, _path: &str, _directory: &Directory) -> Result<WalkAction, HarnessError> {
        Ok(WalkAction::Continue)
    }
    fn leave(&mut self) {}
    fn unavailable(&mut self) {}
    /// One entry the walk reached, with the stat the directory stream already
    /// took for it — kind, size and modification time, no second call. A
    /// caller that addressed one file directly states the same fields itself.
    fn entry(
        &mut self,
        path: &str,
        kind: EntryKind,
        size: u64,
        mtime: Option<SystemTime>,
        directory: &Directory,
    ) -> Result<WalkAction, HarnessError>;
}

impl Workspace {
    pub(super) fn walk_stream(
        &self,
        root: &str,
        visitor: &mut dyn WalkVisitor,
    ) -> Result<(), HarnessError> {
        self.walk_held(root, Directory::open(self, root)?, visitor)
    }

    pub(super) fn walk_held(
        &self,
        root: &str,
        directory: Directory,
        visitor: &mut dyn WalkVisitor,
    ) -> Result<(), HarnessError> {
        descend(self, root, 0, directory, visitor).map(|_| ())
    }

    /// Compatibility wrapper: the same traversal, with the original strict cap.
    pub(crate) fn walk(
        &self,
        max_visited: usize,
        visit: &mut dyn FnMut(&str, EntryKind, u64) -> Result<(), HarnessError>,
    ) -> Result<(), HarnessError> {
        struct Strict<'a> {
            count: usize,
            max: usize,
            visit: &'a mut dyn FnMut(&str, EntryKind, u64) -> Result<(), HarnessError>,
        }
        impl WalkVisitor for Strict<'_> {
            fn seen(&mut self) -> Result<bool, HarnessError> {
                self.count += 1;
                if self.count > self.max {
                    return Err(HarnessError::BoundsExceeded {
                        path: String::new(),
                        found: self.count as u64,
                        max: self.max as u64,
                    });
                }
                Ok(true)
            }
            fn entry(
                &mut self,
                path: &str,
                kind: EntryKind,
                size: u64,
                mtime: Option<SystemTime>,
                directory: &Directory,
            ) -> Result<WalkAction, HarnessError> {
                let _ = (mtime, directory);
                (self.visit)(path, kind, size)?;
                Ok(WalkAction::Continue)
            }
        }
        self.walk_stream(
            "",
            &mut Strict {
                count: 0,
                max: max_visited,
                visit,
            },
        )
    }
}

fn descend(
    workspace: &Workspace,
    path: &str,
    depth: usize,
    mut directory: Directory,
    visitor: &mut dyn WalkVisitor,
) -> Result<bool, HarnessError> {
    if depth > 64 {
        return Err(HarnessError::BoundsExceeded {
            path: String::new(),
            found: depth as u64,
            max: 64,
        });
    }
    match visitor.enter(path, &directory)? {
        WalkAction::Prune => return Ok(false),
        WalkAction::Stop => return Ok(true),
        WalkAction::Continue => {}
    }
    let result = (|| {
        loop {
            if !visitor.advance() {
                return Ok(true);
            }
            let Some(entry) = directory.next()? else {
                break;
            };
            if !visitor.seen()? {
                return Ok(true);
            }
            if entry.kind == EntryKind::Symlink {
                continue;
            }
            let relative = if path.is_empty() {
                entry.name
            } else {
                format!("{path}/{}", entry.name)
            };
            // Exactly the existing containment owner: a name the walk cannot
            // address is skipped, and anything else is reported unavailable.
            if let Err(error) = workspace.target(&relative, false) {
                if !matches!(error, HarnessError::InvalidPath { .. }) {
                    visitor.unavailable();
                }
                continue;
            }
            match visitor.entry(&relative, entry.kind, entry.size, entry.mtime, &directory)? {
                WalkAction::Stop => return Ok(true),
                WalkAction::Prune => continue,
                WalkAction::Continue => {}
            }
            if entry.kind == EntryKind::Dir {
                let name = relative.rsplit('/').next().unwrap_or(&relative);
                let child = directory.child(workspace, &relative, name)?;
                if descend(workspace, &relative, depth + 1, child, visitor)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    })();
    visitor.leave();
    result
}

#[cfg(all(test, unix))]
#[path = "walk_tests.rs"]
mod tests;
